import { BrowserWindow } from "electron";
import { registerImportExportHandlers } from "@/ipc/importExport/index";
import { registerApplicationHandlers } from "@/ipc/applicationHandlers";

import { registerTlsHandlers } from "@/ipc/handlers/tlsHandlers";
import { registerSyncHandlers } from "@/ipc/handlers/syncHandlers";
import { registerFolderHandlers } from "@/ipc/handlers/folderHandlers";
import { registerSoapHandlers } from "@/ipc/handlers/soapHandlers";
import { registerGraphqlHandlers } from "@/ipc/handlers/graphqlHandlers";
import { registerGrpcHandlers } from "@/ipc/handlers/grpcHandlers";
import { registerSystemHandlers } from "@/ipc/handlers/systemHandlers";
import { registerCrudHandlers } from "@/ipc/handlers/crudHandlers";
import { registerRunnerHandlers } from "@/ipc/handlers/runnerHandlers";
import { registerCoreHandlers } from "@/ipc/handlers/coreHandlers";

import { onSyncStatusChange } from "@/sync/syncManager";
import { logEmitter, RequestLogEntry } from "@/proxy/logEmitter";

export function registerIpcHandlers(): void {
  registerImportExportHandlers();
  registerApplicationHandlers();

  // Forward sync status events to every open window
  onSyncStatusChange((wsId, state) => {
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) w.webContents.send("sync:status", { wsId, ...state });
    });
  });

  // Forward request log entries to every open window
  logEmitter.on("request", (entry: RequestLogEntry) => {
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) w.webContents.send("log:entry", entry);
    });
  });

  // Forward streaming log chunks to every open window
  logEmitter.on("chunk", (chunk: { logId: string; chunk: string; done: boolean }) => {
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) w.webContents.send("log:chunk", chunk);
    });
  });

  // Forward server errors to every open window
  logEmitter.on("server-error", (error: string) => {
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) w.webContents.send("server:error", error);
    });
  });

  // Register all categorized handlers
  registerTlsHandlers();
  registerSyncHandlers();
  registerFolderHandlers();
  registerSoapHandlers();
  registerGraphqlHandlers();
  registerGrpcHandlers();
  registerSystemHandlers();
  registerCrudHandlers();
  registerRunnerHandlers();
  registerCoreHandlers();
}
