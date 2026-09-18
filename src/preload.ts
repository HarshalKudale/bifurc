import { contextBridge, ipcRenderer } from "electron";

import { createClient } from "@bifurc/client";

import { createIpcTransport } from "./ipcTransport";

/**
 * P6 work item 1, step 2 — the shell's first RPC client.
 *
 * `plan/07`'s step 2 routes **one** method (`config:get`) through the RPC path and leaves the other
 * 143 on the legacy `ipcRenderer.invoke` channels. That is why this file still looks like a
 * hand-written channel table below: it *is* one, and it is meant to be, until step 3.
 *
 * The point of building the whole client for one method is to prove the seam carries real traffic.
 * A bridge that is only exercised by a test is not evidence that the shell can be moved onto it —
 * and the failure modes here (a `require` that does not resolve, an error code that does not survive
 * the hop, a subscription attempted too early) are all invisible to unit tests and all fatal in the
 * app. One method is enough to expose every one of them, while keeping the blast radius to a single
 * caller if the bridge is wrong.
 *
 * ## Why `local` is built here rather than being another `ipcRenderer` table
 *
 * `createClient` requires it: `ClientLocal` is the 13 keys that act on **the machine the client runs
 * on** — native pickers, the OS trust store, theme, zoom, launch state. They are not RPC and must
 * never be: `tlsInstallCA` mutates the *user's* trust store, so routing it would let an engine
 * install a root certificate on the client. The client takes them as a dependency rather than
 * guessing, which is also what makes it impossible to route one by accident.
 *
 * Each of the 13 is the same channel this file already used, so nothing changes behaviourally: the
 * implementations below are moved verbatim from the `api` object further down. They are duplicated
 * for the length of step 2 — the client needs them, and the exposed surface still needs them — and
 * step 3 collapses the two.
 *
 * `platform` is the exception: it never crossed IPC at all. It is `process.platform`, read in the
 * preload, because the renderer cannot ask the main process a question whose answer is a constant.
 */
const client = createClient(createIpcTransport(), {
  local: {
    openExternal: (url: string) => ipcRenderer.invoke("shell:openExternal", url),
    setTitleBarOverlay: (color: string, symbolColor: string) =>
      ipcRenderer.invoke("shell:setTitleBarOverlay", color, symbolColor),
    getTheme: () => ipcRenderer.invoke("theme:get"),
    setTheme: (themeId: string) => ipcRenderer.invoke("theme:set", themeId),
    tlsInstallCA: () => ipcRenderer.invoke("tls:installCA"),
    openFileDialog: () => ipcRenderer.invoke("dialog:openFile"),
    pickFilePath: (title: string, filters?: unknown) => ipcRenderer.invoke("dialog:pickFilePath", title, filters),
    pickFolderPath: (title: string) => ipcRenderer.invoke("dialog:pickFolderPath", title),
    platform: process.platform,
    isFirstLaunch: () => ipcRenderer.invoke("app:isFirstLaunch"),
    completeFirstLaunch: () => ipcRenderer.invoke("app:completeFirstLaunch"),
    getZoomLevel: () => ipcRenderer.invoke("zoom:get"),
    setZoomLevel: (level: number) => ipcRenderer.invoke("zoom:set", level),
  },
});

contextBridge.exposeInMainWorld("api", {
  checkUpdate: () => ipcRenderer.invoke("app:checkUpdate"),
  // ── P6 step 2: the single method on the RPC path ──────────────────────────
  //
  // Was `ipcRenderer.invoke("config:get")`. It now goes
  // `client → ipcTransport → ipcRenderer.invoke("engine:rpc") → rpcBridge → registry.invoke("config.get")`,
  // and the legacy `config:get` channel stays registered and serving — the two paths coexist on
  // purpose so the phase can be reverted without unpicking anything.
  //
  // The observable contract is unchanged: `config.get` is already routed through the registry on the
  // main side (`coreHandlers.ts:116`), so both paths reach the same handler and resolve the same
  // value. What differs is only what happens on *failure* — the RPC path rebuilds the `EngineError`
  // instead of losing its code to Electron's flattened rejection.
  getConfig: () => client.getConfig(),
  loadEntity: (wsId: string, kind: string, id: string) => ipcRenderer.invoke("entity:load", wsId, kind, id),
  setEntityEnabled: (wsId: string, kind: string, id: string, enabled: boolean) => ipcRenderer.invoke("entity:setEnabled", wsId, kind, id, enabled),
  saveConfig: (config: unknown) => ipcRenderer.invoke("config:save", config),
  getImportExportFormats: () => ipcRenderer.invoke("importExport:formats"),
  exportData: (req: unknown) => ipcRenderer.invoke("importExport:export", req),
  preflightImport: (req: unknown) => ipcRenderer.invoke("importExport:preflight", req),
  importData: (req: unknown) => ipcRenderer.invoke("importExport:import", req),
  discoverServices: () => ipcRenderer.invoke("services:discover"),
  addMapping: (mapping: unknown) => ipcRenderer.invoke("mapping:add", mapping),
  updateMapping: (mapping: unknown) => ipcRenderer.invoke("mapping:update", mapping),
  deleteMapping: (id: string) => ipcRenderer.invoke("mapping:delete", id),
  addRule: (rule: unknown) => ipcRenderer.invoke("rule:add", rule),
  updateRule: (rule: unknown) => ipcRenderer.invoke("rule:update", rule),
  deleteRule: (id: string) => ipcRenderer.invoke("rule:delete", id),
  addMock: (mock: unknown) => ipcRenderer.invoke("mock:add", mock),
  updateMock: (mock: unknown) => ipcRenderer.invoke("mock:update", mock),
  deleteMock: (id: string) => ipcRenderer.invoke("mock:delete", id),
  addRequest: (req: unknown) => ipcRenderer.invoke("request:add", req),
  updateRequest: (req: unknown) => ipcRenderer.invoke("request:update", req),
  deleteRequest: (id: string) => ipcRenderer.invoke("request:delete", id),
  addWsConnection: (conn: unknown) => ipcRenderer.invoke("ws:add", conn),
  updateWsConnection: (conn: unknown) => ipcRenderer.invoke("ws:update", conn),
  deleteWsConnection: (id: string) => ipcRenderer.invoke("ws:delete", id),
  addFolder: (kind: string, folder: unknown) => ipcRenderer.invoke("folder:add", kind, folder),
  renameFolder: (kind: string, id: string, name: string) => ipcRenderer.invoke("folder:rename", kind, id, name),
  moveFolder: (kind: string, id: string, parentId: string | null) => ipcRenderer.invoke("folder:move", kind, id, parentId),
  deleteFolder: (kind: string, id: string) => ipcRenderer.invoke("folder:delete", kind, id),
  addEnvironment: (env: unknown) => ipcRenderer.invoke("env:add", env),
  updateEnvironment: (env: unknown) => ipcRenderer.invoke("env:update", env),
  deleteEnvironment: (id: string) => ipcRenderer.invoke("env:delete", id),
  setActiveEnvironment: (id: string | null) => ipcRenderer.invoke("env:setActive", id),
  addWorkspace: (name: string) => ipcRenderer.invoke("workspace:add", name),
  renameWorkspace: (id: string, name: string) => ipcRenderer.invoke("workspace:rename", id, name),
  deleteWorkspace: (id: string) => ipcRenderer.invoke("workspace:delete", id),
  setActiveWorkspace: (id: string) => ipcRenderer.invoke("workspace:setActive", id),
  replayRequest: (method: string, url: string, headers: unknown, body: string) =>
    ipcRenderer.invoke("request:replay", method, url, headers, body),
  proxyStatus: () => ipcRenderer.invoke("proxy:status"),
  serverStatus: () => ipcRenderer.invoke("server:status"),
  restartServer: () => ipcRenderer.invoke("server:restart"),
  stopServer: () => ipcRenderer.invoke("server:stop"),
  startServer: () => ipcRenderer.invoke("server:start"),
  openExternal: (url: string) => ipcRenderer.invoke("shell:openExternal", url),
  setTitleBarOverlay: (color: string, symbolColor: string) => ipcRenderer.invoke("shell:setTitleBarOverlay", color, symbolColor),
  listAudit: (opts?: unknown) => ipcRenderer.invoke("audit:list", opts),
  auditDiff: (commitHash: string, entity: string, entityId: string, wsId: string) =>
    ipcRenderer.invoke("audit:diff", commitHash, entity, entityId, wsId),
  exportAudit: (format: "json" | "csv") => ipcRenderer.invoke("audit:export", format),
  listHistory: (opts: { workspaceId?: string; filePath: string; limit?: number; offset?: number }) =>
    ipcRenderer.invoke("history:list", opts),
  historyDiff: (commitHash: string, filePath: string, wsId: string) =>
    ipcRenderer.invoke("history:diff", commitHash, filePath, wsId),
  syncSetRemote: (wsId: string, remote: string, branch: string) =>
    ipcRenderer.invoke("sync:setRemote", wsId, remote, branch),
  syncDisconnect: (wsId: string) => ipcRenderer.invoke("sync:disconnect", wsId),
  syncPush: (wsId: string) => ipcRenderer.invoke("sync:push", wsId),
  syncPull: (wsId: string) => ipcRenderer.invoke("sync:pull", wsId),
  syncGetState: (wsId: string) => ipcRenderer.invoke("sync:getState", wsId),
  syncSetAutoSync: (wsId: string, enabled: boolean) => ipcRenderer.invoke("sync:setAutoSync", wsId, enabled),
  onSyncStatus: (cb: (state: { wsId: string; status: string; error?: string | null; updatedIds?: string[] }) => void) => {
    const handler = (_: unknown, state: unknown) => cb(state as any);
    ipcRenderer.on("sync:status", handler);
    return () => ipcRenderer.off("sync:status", handler);
  },
  publishEntity: (wsId: string, paths: string[], message?: string) => ipcRenderer.invoke("entity:publish", wsId, paths, message),
  publishFolder: (wsId: string, kind: string, folderName: string | null) => ipcRenderer.invoke("folder:publish", wsId, kind, folderName),
  restoreEntity: (wsId: string, relPath: string) => ipcRenderer.invoke("entity:restore", wsId, relPath),
  gitGetDiff: (wsId: string, relPath: string) => ipcRenderer.invoke("git:diff", wsId, relPath),
  gitDiscard: (wsId: string, relPath: string) => ipcRenderer.invoke("git:discard", wsId, relPath),
  gitSync: (wsId: string, paths: string[], message?: string) => ipcRenderer.invoke("git:sync", wsId, paths, message),
  gitGetHistory: (wsId: string, relPath: string, opts?: { limit?: number; offset?: number }) => ipcRenderer.invoke("git:history", wsId, relPath, opts),
  getEntitySyncStatus: (wsId: string) => ipcRenderer.invoke("sync:getEntityStatus", wsId),
  onEntitySyncStatus: (cb: (data: { wsId: string; status: Record<string, string> }) => void) => {
    const handler = (_: unknown, data: unknown) => cb(data as any);
    ipcRenderer.on("sync:entityStatus", handler);
    return () => ipcRenderer.off("sync:entityStatus", handler);
  },
  executeScript: (opts: unknown) => ipcRenderer.invoke("script:execute", opts),
  onLogEntry: (cb: (entry: unknown) => void) => {
    const handler = (_: unknown, entry: unknown) => cb(entry);
    ipcRenderer.on("log:entry", handler);
    return () => ipcRenderer.off("log:entry", handler);
  },
  onServerError: (cb: (error: string) => void) => {
    const handler = (_: unknown, error: unknown) => cb(error as string);
    ipcRenderer.on("server:error", handler);
    return () => ipcRenderer.off("server:error", handler);
  },
  healthbarGetServices: (wsId: string) => ipcRenderer.invoke("healthbar:getServices", wsId),
  healthbarSaveServices: (wsId: string, services: unknown[]) => ipcRenderer.invoke("healthbar:saveServices", wsId, services),
  healthbarCheckUrl: (url: string) => ipcRenderer.invoke("healthbar:checkUrl", url),
  // Webhooks
  addWebhook: (hook: unknown) => ipcRenderer.invoke("webhook:add", hook),
  updateWebhook: (hook: unknown) => ipcRenderer.invoke("webhook:update", hook),
  deleteWebhook: (id: string) => ipcRenderer.invoke("webhook:delete", id),
  registerActiveWebhook: (webhookId: string, urlSuffix: string) => ipcRenderer.invoke("webhook:registerActive", webhookId, urlSuffix),
  unregisterActiveWebhook: (webhookId: string) => ipcRenderer.invoke("webhook:unregisterActive", webhookId),
  getWebhookServerStatus: () => ipcRenderer.invoke("webhookServer:status"),
  webhookServerStatus: () => ipcRenderer.invoke("webhookServer:status"),
  startWebhookServer: () => ipcRenderer.invoke("webhookServer:start"),
  stopWebhookServer: () => ipcRenderer.invoke("webhookServer:stop"),
  onWebhookPayload: (cb: (payload: unknown) => void) => {
    const handler = (_: unknown, payload: unknown) => cb(payload);
    ipcRenderer.on("webhook:payload", handler);
    return () => ipcRenderer.off("webhook:payload", handler);
  },
  // ── SOAP ──────────────────────────────────────────────────────────────────
  addSoapRequest: (req: unknown) => ipcRenderer.invoke("soap:addRequest", req),
  updateSoapRequest: (req: unknown) => ipcRenderer.invoke("soap:updateRequest", req),
  deleteSoapRequest: (id: string) => ipcRenderer.invoke("soap:deleteRequest", id),
  addSoapMock: (mock: unknown) => ipcRenderer.invoke("soap:addMock", mock),
  updateSoapMock: (mock: unknown) => ipcRenderer.invoke("soap:updateMock", mock),
  deleteSoapMock: (id: string) => ipcRenderer.invoke("soap:deleteMock", id),
  addWsdl: (wsdl: unknown) => ipcRenderer.invoke("soap:addWsdl", wsdl),
  deleteWsdl: (id: string) => ipcRenderer.invoke("soap:deleteWsdl", id),
  listWsdls: () => ipcRenderer.invoke("soap:listWsdls"),
  soapFetchWsdl: (url: string) => ipcRenderer.invoke("soap:fetchWsdl", url),
  soapExecute: (endpointUrl: string, soapAction: string, headers: unknown, body: string) =>
    ipcRenderer.invoke("soap:execute", { endpointUrl, soapAction, headers, body }),
  // ── GraphQL ───────────────────────────────────────────────────────────────
  addGraphQLRequest: (req: unknown) => ipcRenderer.invoke("graphql:addRequest", req),
  updateGraphQLRequest: (req: unknown) => ipcRenderer.invoke("graphql:updateRequest", req),
  deleteGraphQLRequest: (id: string) => ipcRenderer.invoke("graphql:deleteRequest", id),
  addGraphQLMock: (mock: unknown) => ipcRenderer.invoke("graphql:addMock", mock),
  updateGraphQLMock: (mock: unknown) => ipcRenderer.invoke("graphql:updateMock", mock),
  deleteGraphQLMock: (id: string) => ipcRenderer.invoke("graphql:deleteMock", id),
  addGraphQLSchema: (schema: unknown) => ipcRenderer.invoke("graphql:addSchema", schema),
  deleteGraphQLSchema: (id: string) => ipcRenderer.invoke("graphql:deleteSchema", id),
  listGraphQLSchemas: () => ipcRenderer.invoke("graphql:listSchemas"),
  graphqlIntrospect: (url: string, headers: unknown) => ipcRenderer.invoke("graphql:introspect", { url, headers }),
  graphqlExecute: (url: string, headers: unknown, query: string, variables: string, operationName: string) =>
    ipcRenderer.invoke("graphql:execute", { url, headers, query, variables, operationName }),
  // ── gRPC ──────────────────────────────────────────────────────────────────
  addGrpcRequest: (req: unknown) => ipcRenderer.invoke("grpc:addRequest", req),
  updateGrpcRequest: (req: unknown) => ipcRenderer.invoke("grpc:updateRequest", req),
  deleteGrpcRequest: (id: string) => ipcRenderer.invoke("grpc:deleteRequest", id),
  addGrpcMock: (mock: unknown) => ipcRenderer.invoke("grpc:addMock", mock),
  updateGrpcMock: (mock: unknown) => ipcRenderer.invoke("grpc:updateMock", mock),
  deleteGrpcMock: (id: string) => ipcRenderer.invoke("grpc:deleteMock", id),
  addProtoFile: (proto: unknown) => ipcRenderer.invoke("grpc:addProto", proto),
  deleteProtoFile: (id: string) => ipcRenderer.invoke("grpc:deleteProto", id),
  listProtoFiles: () => ipcRenderer.invoke("grpc:listProtos"),
  grpcExecute: (serverAddress: string, serviceName: string, methodName: string, requestBody: string, metadata: unknown, protoFileId: string | null, useReflection: boolean) =>
    ipcRenderer.invoke("grpc:execute", { serverAddress, serviceName, methodName, requestBody, metadata, protoFileId, useReflection }),
  grpcReflect: (serverAddress: string) => ipcRenderer.invoke("grpc:reflect", { serverAddress }),
  grpcMockServerStatus: () => ipcRenderer.invoke("grpc:mockServerStatus"),
  grpcStartMockServer: () => ipcRenderer.invoke("grpc:startMockServer"),
  grpcStopMockServer: () => ipcRenderer.invoke("grpc:stopMockServer"),
  tlsImportCert: () => ipcRenderer.invoke("tls:importCert"),
  tlsImportKey: () => ipcRenderer.invoke("tls:importKey"),
  tlsRemoveCert: () => ipcRenderer.invoke("tls:removeCert"),
  tlsGenerate: () => ipcRenderer.invoke("tls:generate"),
  tlsInstallCA: () => ipcRenderer.invoke("tls:installCA"),
  tlsExportCert: () => ipcRenderer.invoke("tls:exportCert"),
  tlsCertStatus: () => ipcRenderer.invoke("tls:certStatus"),
  onCompanionRefresh: (cb: () => void) => {
    const handler = () => cb();
    ipcRenderer.on("companion:refresh", handler);
    return () => ipcRenderer.off("companion:refresh", handler);
  },
  openFileDialog: () => ipcRenderer.invoke("dialog:openFile"),
  pickFilePath: (title: string, filters?: unknown) => ipcRenderer.invoke("dialog:pickFilePath", title, filters),
  pickFolderPath: (title: string) => ipcRenderer.invoke("dialog:pickFolderPath", title),
  platform: process.platform,
  onLogChunk: (cb: (chunk: unknown) => void) => {
    const handler = (_: unknown, chunk: unknown) => cb(chunk);
    ipcRenderer.on("log:chunk", handler);
    return () => ipcRenderer.off("log:chunk", handler);
  },
  shareCaptureJson: (entries: unknown[], suggestedName?: string) =>
    ipcRenderer.invoke("capture:shareJson", entries, suggestedName),

  // ── First-launch ───────────────────────────────────────────────────────────
  isFirstLaunch: () => ipcRenderer.invoke("app:isFirstLaunch"),
  completeFirstLaunch: () => ipcRenderer.invoke("app:completeFirstLaunch"),
  // ── Collection Runner ─────────────────────────────────────────────────────
  saveRunnerReport: (wsId: string, report: unknown) => ipcRenderer.invoke("runner:saveReport", wsId, report),
  getRunHistory: (wsId: string, folderId: string) => ipcRenderer.invoke("runner:getHistory", wsId, folderId),
  exportRunnerReport: (report: unknown) => ipcRenderer.invoke("runner:exportReport", report),
  saveRunnerConfig: (wsId: string, folderId: string, config: unknown) => ipcRenderer.invoke("runner:saveConfig", wsId, folderId, config),
  loadRunnerConfig: (wsId: string, folderId: string) => ipcRenderer.invoke("runner:loadConfig", wsId, folderId),
  listRunnerFolderIds: (wsId: string) => ipcRenderer.invoke("runner:listFolderIds", wsId),

  // ── Zoom ────────────────────────────────────────────────────────────────────
  getZoomLevel: () => ipcRenderer.invoke("zoom:get"),
  setZoomLevel: (level: number) => ipcRenderer.invoke("zoom:set", level),

  // ── Theme ───────────────────────────────────────────────────────────────────
  getTheme: () => ipcRenderer.invoke("theme:get"),
  setTheme: (themeId: string) => ipcRenderer.invoke("theme:set", themeId),
});
