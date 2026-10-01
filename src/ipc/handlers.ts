import { registerSystemHandlers } from "@/ipc/handlers/systemHandlers";
import { registerRpcBridge } from "@/ipc/rpcBridge";

/**
 * The desktop shell's own registration: the channels only Electron can serve, and the RPC bridge.
 *
 * The engine's commands and event wiring are **not** here: `main.ts` starts the same `createWorkspaceHost()`
 * the standalone engine runs, and that registers them. A shell that registered its own copy is the
 * duplicated composition the sidecar split cannot afford.
 */
export function registerIpcHandlers(): void {
  // Shell-only: the update check talks to GitHub on behalf of this app build, not the workspace.
  registerSystemHandlers();
  registerRpcBridge();
}
