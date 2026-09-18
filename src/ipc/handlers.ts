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
import { registerWebhookCommands } from "@bifurc/engine/proxy/webhookCommands";
import { registerConfigCommands } from "@bifurc/engine/store/configCommands";
import { registerMiscCommands } from "@bifurc/engine/miscCommands";
import { registerRunnerCommands } from "@bifurc/engine/runner/runnerCommands";
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
  // P6 finding 5, step 3b-2 (second slice): `config.save` and the three `workspace.*` lifecycle
  // commands. Same class of bug as the six above — served only by shell `ipcMain.handle` bodies, so
  // `registry.invoke()` answered `UNKNOWN_COMMAND`. `config.save` is the more consequential of the
  // two halves: it is the settings path, and it carries the server restart.
  registerConfigCommands(commandRegistry);
  // P6 finding 5, step 3b-2 (third slice): the two `webhook.*` active-registration commands and the
  // three `webhookServer.*` lifecycle commands. Same class of bug as the ten above — served only by
  // `ipcMain.handle` bodies in `crudHandlers.ts`, so `registry.invoke()` answered `UNKNOWN_COMMAND`,
  // and `@bifurc/client` advertised five methods it could not deliver. The listening server, its port
  // and its error state are all engine state in `proxy/webhookServer.ts`.
  //
  // `webhookServer.start` is the one handler in this batch that is not a literal copy: the shell body
  // ignored its (non-existent) argument, while `WebhookServerStartParams` declares an optional `port`.
  // The engine half honours it and falls back to `cfg.webhookPort ?? 9101`, so every existing caller
  // — all of which pass nothing — gets exactly the old behaviour.
  registerWebhookCommands(commandRegistry);
  // P6 finding 5, step 3b-2 (fourth slice): the five `misc.ts` commands — `healthbar.getServices` /
  // `healthbar.saveServices` / `healthbar.checkUrl`, `request.replay` and `script.execute`. All five
  // were `ipcMain.handle` bodies in `coreHandlers.ts`; every function they call already lives in the
  // engine. `healthbar.checkUrl` is the one body that could not be copied literally: its shell version
  // used `require("https")`, and `require` does not exist in the engine's ESM build output.
  registerMiscCommands(commandRegistry);
  // P6 finding 5, step 3b-2 (fifth slice): `runner.saveReport`. Its two siblings
  // (`runner.saveConfig` / `runner.loadConfig`) are `BLOCKED` by the frozen protocol and stay on their
  // legacy channels; `runner.exportReport` had already moved.
  registerRunnerCommands(commandRegistry);

  registerImportExportHandlers();
  registerApplicationHandlers();

  // Forward sync status changes onto the bus (P2 work item 2, site #1). `log:entry` /
  // `log:chunk` / `server:error` need no forwarder *here* — `logEmitter` is already
  // Electron-free, so `wireLogEventsToBus()` below puts them on the bus directly.
  onSyncStatusChange((wsId, state) => {
    bus.emitTyped("sync.status", { wsId, ...state });
  });

  // P6 finding 1: the three log events must **also** reach the bus, or a transport-only P6 delivers
  // nothing to `onLogEntry` / `onLogChunk` / `onServerError` and the capture panel, the request-log
  // panel and the server-error banner all go dead — with every unit test still green, because they
  // test the bridge's *mapping* rather than its *traffic*.
  //
  // The bus is now the **only** path for these three, since step 3c deleted `eventBridge.ts` — the
  // file that used to read `logEmitter` directly and broadcast the legacy `log:entry` /
  // `log:chunk` / `server:error` channels. Nothing listens on those channels any more (the preload
  // subscribes through the RPC bridge), so this call is what keeps the capture panel, the request-log
  // panel and the server-error banner alive rather than merely additive.
  //
  // On the shell's path specifically, because the shell uses the module singletons and never calls
  // `createEngine()`; wiring it only in the factory would leave today's only client unserved.
  wireLogEventsToBus();

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
