/**
 * The `script.execute` command — run a user-authored script in the sandbox and return its result.
 *
 * ## Where this file came from
 *
 * It is one of the three destinations of `packages/engine/src/miscCommands.ts`, which
 * `Engine_Module_Plan.md` §4.5 dissolved in the engine-layout's phase 7. §4.5's reason for putting it
 * here rather than anywhere else is one line: it calls `executeIpcScript`, which lives in this module's
 * `scriptExecutor.ts`. The command is the sandbox's entry point, so it belongs beside the sandbox.
 *
 * The other two destinations: `request.replay` → `rest/commands.ts`, the three `healthbar.*` commands →
 * `workspace/healthbarCommands.ts`.
 *
 * ## Why this moved out of the shell (P6 finding 5)
 *
 * It was an `ipcMain.handle` body in `src/ipc/handlers/coreHandlers.ts`, reachable only while the shell
 * and the engine are one process — which is the whole reason a containerised host could not run a
 * script. `transport/auth/scopes.ts` had already assigned it the `execute` scope before this module
 * existed, and that table is written against `@bifurc/protocol` command names, so it is independent
 * evidence this was always an engine command — the same signal `serverCommands.ts` found for its six.
 *
 * ## The body is moved verbatim
 *
 * `executeIpcScript` owns the sandbox, the context and the timeout; the command body is a one-line
 * delegation and stayed that way. Nothing here needed the `require`→static-import fix that
 * `healthbar.checkUrl` needed, because the sandbox's own module access is already handled inside
 * `scriptExecutor.ts`.
 */
import type { ScriptExecuteParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { executeIpcScript } from "./scriptExecutor";

export function registerMockCommands(registry: CommandRegistry): void {
    registry.register("script.execute", (params: ScriptExecuteParams) => executeIpcScript(params));
}
