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
import { bus } from "@/events/bus";
import { wireEventBridge } from "@/ipc/eventBridge";

export function registerIpcHandlers(): void {
  registerImportExportHandlers();
  registerApplicationHandlers();

  // Forward sync status changes onto the bus (P2 work item 2, site #1). `log:entry` /
  // `log:chunk` / `server:error` no longer need a forwarder here — `logEmitter` is already
  // Electron-free, so the shell's `eventBridge.ts` subscribes to it directly.
  onSyncStatusChange((wsId, state) => {
    bus.emitTyped("sync.status", { wsId, ...state });
  });

  // Pre-P6: wire the temporary shell bridge so the renderer keeps receiving these events over
  // the existing `ipcRenderer.on(...)` channels, unchanged, while every emission site below is
  // converted to the bus one at a time. Deleted wholesale once P6 lands the real transport.
  wireEventBridge();

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
