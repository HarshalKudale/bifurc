import { ipcMain } from "electron";
import type { SoapFetchWsdlParams, SoapExecuteParams } from "@bifurc/protocol";
import { registerEntityCrudHandlers, registerSimpleEntityHandlers } from "@/ipc/handlers/entityCrudFactory";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

interface SavedSoapRequest {
  id: string; name: string; endpointUrl: string; soapAction: string;
  headers: Record<string, string>; body: string; wsdlId?: string | null;
  operationName?: string; preScript?: string; postScript?: string;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedSoapMock {
  id: string; name: string; enabled: boolean; endpointPattern: string; useRegex: boolean;
  soapActionPattern: string; operationName?: string;
  responseStatus: number; responseHeaders: Record<string, string>; responseBody: string;
  responseDelay?: number; wsdlId?: string | null;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedWsdl {
  id: string; name: string; content: string; sourceUrl?: string;
  importedAt: number; createdAt: number; workspaceId: string;
}

// ## Channels only — the registry half now lives in the engine
//
// `soap.fetchWsdl` / `soap.execute` used to be registered at **module scope** in this file, which is
// the `plan/07` §5c defect: a containerised engine (P9) answered `UNKNOWN_COMMAND` for both. Step 3e
// slice 4 (2026-09-19) moved them into `packages/engine/src/protocols/soapCommands.ts` behind
// `registerSoapCommands()`, called from `src/ipc/handlers.ts`.
//
// Every channel below **delegates** through `commandRegistry.invoke()` rather than duplicating the
// body — unlike `runnerHandlers.ts`, where three of the five are self-contained copies. That
// distinction decides what breaks: a delegating channel is only ever as alive as the registry behind
// it, so a suite that registers this handler group by hand **must** also call
// `registerSoapCommands(commandRegistry)`, or the channel exists with nothing behind it. The failure
// is `No handler registered` at *invocation* time, so nothing catches it at build time.
//
// The three CRUD groups are a different thing and stay: `soap:addRequest/updateRequest/deleteRequest`,
// `soap:addMock/updateMock/deleteMock` and `soap:addWsdl/deleteWsdl/listWsdls` route through the CRUD
// collapse — see `entityCrudFactory.ts` and `plan/03-phase-2-engine-extraction.md` work item 7's
// tenth batch. The last group goes through the smaller `registerSimpleEntityHandlers()` path because
// WSDLs don't track a `configKey` array the way every `CrudFactoryOpts` kind does (they are written
// straight to disk, nothing mirrors them into `AppConfig`) and have no "update" concept.
//
// P2 work item 7 — `soap.fetchWsdl`/`execute` convert directly: the params match the frozen schema
// exactly, so no cast or translation is needed.
const ctx = { bus };

export function registerSoapHandlers() {
  registerEntityCrudHandlers<SavedSoapRequest>({
    ipcAdd: "soap:addRequest", ipcUpdate: "soap:updateRequest", ipcDelete: "soap:deleteRequest",
    kind: "soapRequests",
    configKey: "soapRequests" as any,
    folderConfigKey: "soapRequestFolders" as any,
    getNameEntry: (req) => ({ name: req.name, endpointUrl: req.endpointUrl, soapAction: req.soapAction }),
  });

  registerEntityCrudHandlers<SavedSoapMock>({
    ipcAdd: "soap:addMock", ipcUpdate: "soap:updateMock", ipcDelete: "soap:deleteMock",
    kind: "soapMocks",
    configKey: "soapMocks" as any,
    folderConfigKey: "soapMockFolders" as any,
    hasEnabledState: true,
    validate: (mock) => {
      if (!mock.endpointPattern || !mock.endpointPattern.trim()) throw new Error("endpointPattern is required for SOAP mocks");
    },
    getNameEntry: (mock) => ({ name: mock.name, soapActionPattern: mock.soapActionPattern }),
  });

  registerSimpleEntityHandlers<SavedWsdl>({
    kind: "wsdls",
    ipcAdd: "soap:addWsdl", ipcDelete: "soap:deleteWsdl", ipcList: "soap:listWsdls",
  });

  ipcMain.handle("soap:fetchWsdl", (_e, url: string) =>
    commandRegistry.invoke("soap.fetchWsdl", { url }, ctx));

  ipcMain.handle("soap:execute", (_e, params: SoapExecuteParams) =>
    commandRegistry.invoke("soap.execute", params, ctx));
}
