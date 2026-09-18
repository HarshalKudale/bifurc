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
import { bus, wireLogEventsToBus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { registerBlobCommands } from "@bifurc/engine/blob/commands";
import { registerImportExportCommands } from "@bifurc/engine/importExport/commands";
import { registerFileOpsCommands } from "@bifurc/engine/fileOps/commands";
import { registerCertCommands } from "@bifurc/engine/proxy/certCommands";
import { registerServerCommands } from "@bifurc/engine/proxy/serverCommands";
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
  // P6 finding 5, step 3b-2 (first slice): `server.status` / `server.start` / `server.stop` /
  // `server.restart` / `proxy.status` / `services.discover`. These six were served **only** by
  // `ipcMain.handle` bodies in `systemHandlers.ts` and `coreHandlers.ts`, so `registry.invoke()`
  // answered `UNKNOWN_COMMAND` for all of them — which is why `@bifurc/client` advertised six
  // methods it could not deliver against the real engine. The proxy server and its port/error state
  // are engine state, not shell state.
  //
  // Those shell bodies **stay as they are** for now, which is deliberate rather than an oversight:
  // re-pointing them at the registry would make them depend on this function having run, and two
  // suites register handler *groups* without it. See `serverCommands.ts`'s header for the full trap.
  registerServerCommands(commandRegistry);

  registerImportExportHandlers();
  registerApplicationHandlers();

  // Forward sync status changes onto the bus (P2 work item 2, site #1). `log:entry` /
  // `log:chunk` / `server:error` need no forwarder *here* — `logEmitter` is already
  // Electron-free, so the shell's `eventBridge.ts` subscribes to it directly.
  onSyncStatusChange((wsId, state) => {
    bus.emitTyped("sync.status", { wsId, ...state });
  });

  // P6 finding 1: the three log events must **also** reach the bus, or a transport-only P6 delivers
  // nothing to `onLogEntry` / `onLogChunk` / `onServerError` and the capture panel, the request-log
  // panel and the server-error banner all go dead — with every unit test still green, because they
  // test the bridge's *mapping* rather than its *traffic*.
  //
  // **Additive, not a replacement.** `eventBridge.ts` subscribes to the *bus* for six events but to
  // `logEmitter` directly for these three, so wiring the bus here cannot double-deliver to the
  // renderer: the legacy channels keep coming from `logEmitter`, and the bus now additionally carries
  // them for the transport. Both paths run side by side until step 3 deletes the legacy one — and
  // this call is precisely what makes that deletion possible instead of silently fatal.
  //
  // On the shell's path specifically, because the shell uses the module singletons and never calls
  // `createEngine()`; wiring it only in the factory would leave today's only client unserved.
  wireLogEventsToBus();

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
