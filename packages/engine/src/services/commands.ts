import type { CommandRegistry } from "../kernel/registry";
import { discoverServices } from "./discovery";

/**
 * `services.discover` — the ports this host is listening on. Not workspace-scoped: it is a property of the
 * host (see the note in `proxy/serverCommands.ts` history), so it takes no `workspaceId`.
 */
export function registerServicesCommands(registry: CommandRegistry): void {
    registry.register("services.discover", () => discoverServices());
}
