import { registerImportExportHandlers } from "@/ipc/importExportHandlers";
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

import { onSyncStatusChange } from "@bifurc/engine/sync/syncManager";
import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { registerBlobCommands } from "@bifurc/engine/blob/commands";
import { registerImportExportCommands } from "@bifurc/engine/importExport/commands";
import { registerFileOpsCommands } from "@bifurc/engine/fileOps/commands";
import { registerCertCommands } from "@bifurc/engine/proxy/certCommands";
import { wireEventBridge } from "@/ipc/eventBridge";
import { registerRpcBridge } from "@/ipc/rpcBridge";

export function registerIpcHandlers(): void {
  // P3: the engine-side command surface the shell is willing to serve. `createEngine()` registers
  // **no** commands by design — the consumer declares what it is willing to serve — so this is
  // where the shell, today's only client, does that. All three are explicit calls rather than
  // module side effects, because `CommandRegistry.register()` throws on a double registration and
  // an explicit call site makes "exactly once" checkable by reading one function.
  registerBlobCommands(commandRegistry);
  registerImportExportCommands(commandRegistry);
  // P3 work items 3–4: `tls.exportCert` / `tls.importCert` / `tls.importKey`,
  // `runner.exportReport`, `audit.export`, `capture.shareJson`.
  registerFileOpsCommands(commandRegistry);
  // P3 work item 5: `tls.generate` / `tls.certStatus` / `tls.removeCert`. Moved here from
  // `handlers/tlsHandlers.ts`, which was serving engine commands from the client half — it works
  // only while the two are the same process. `tls:installCA` did **not** come with them: the host
  // trust store is CLIENT-classified and stays in `clientHandlers.ts`.
  registerCertCommands(commandRegistry);

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

  // P6 work item 1, step 2: the RPC bridge, over `createInProcessTransport(commandRegistry)`.
  //
  // Deliberately last, and deliberately **in addition to** everything above rather than instead of
  // it. `plan/07`'s step 2 routes exactly one method (`config:get`) through the new path and leaves
  // the legacy channels serving the other 143, so a bridge defect costs one method instead of the
  // whole app — and the phase stays revertable, which it would not be if the old path were deleted
  // here. Step 3 is the flip; this is the proof that the seam carries traffic at all.
  //
  // It must come after the `register*Commands(commandRegistry)` calls at the top of this function:
  // the bridge dispatches through the registry, and a command that has not been registered yet is an
  // `UNKNOWN_COMMAND` that only shows up when something finally calls it.
  registerRpcBridge();
}
