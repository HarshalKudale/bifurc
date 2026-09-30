import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { registerAllEngineCommands, wireEngineEvents } from "@bifurc/engine/registerAll";
import { registerSystemHandlers } from "@/ipc/handlers/systemHandlers";
import { registerRpcBridge } from "@/ipc/rpcBridge";

/**
 * The desktop shell's registration: the engine's commands and event wiring, then the shell's own channels.
 *
 * The engine half is **one call each**, the same calls the headless host makes. This file used to list every
 * `register*Commands()` and `register*Kinds()` by hand, beside a test that checked the copy against
 * `registerAll.ts`; the wiring next to the list (`sync.status`, captured traffic) had already drifted — only
 * the shell had it, so a client of the headless host never saw either. A composition that differs from the
 * engine's own is exactly what the sidecar split cannot afford.
 */
export function registerIpcHandlers(): void {
  registerAllEngineCommands(commandRegistry);
  wireEngineEvents();

  // Shell-only: the update check talks to GitHub on behalf of this app build, not the workspace.
  registerSystemHandlers();
  registerRpcBridge();
}
