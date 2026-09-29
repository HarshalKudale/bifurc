/**
 * `request.replay` — send a saved request to its target and return the response.
 *
 * ## Where this file came from
 *
 * It is one of the three destinations of `packages/engine/src/miscCommands.ts`, which
 * `Engine_Module_Plan.md` §4.5 dissolved in the engine-layout's phase 7. §4.5 assigned `request.replay`
 * here because REST is the feature it serves.
 *
 * The other two destinations: `script.execute` → `mocks/commands.ts`, the three `healthbar.*` commands
 * → `workspace/healthbarCommands.ts`.
 *
 * ## What `rest/` is, and what it is not yet
 *
 * §9 is explicit that this module is *mostly a home for future work* and should not be inflated: REST
 * today is one CRUD kind and this one command. The kind is **not** registered here — it stays in the
 * shared `workspace/entities/kinds/crudKinds.ts` table, because §4.4 forbids a feature module owning a
 * private kind list and `requests` is generic CRUD machinery (a JSON entity with a `url`/`method`
 * guard), indistinguishable in shape from `mappings`/`rules`/`mocks`. What `rest/` owns is the
 * *behaviour*: replay.
 *
 * ## Why the implementation is in `utils/`, not here
 *
 * `replayRequest` is shared with the proxy's own routing path (`proxy/routingUtils.ts`). §6 permits
 * `rest/` to import `kernel/`, `utils/` and `workspace/` but not `proxy/`, so per §5.1's rule the
 * shared behaviour was extracted down into `utils/replay.ts` rather than imported sideways. The import
 * below is therefore a `utils/` edge, and the proxy imports the same function from the same place.
 *
 * ## Why this moved out of the shell (P6 finding 5)
 *
 * It was an `ipcMain.handle` body in `src/ipc/handlers/coreHandlers.ts`, reachable only while the shell
 * and the engine are one process. `transport/auth/scopes.ts` had already assigned it a scope before this
 * module existed — the same independent evidence `serverCommands.ts` found for its six.
 *
 * ## The one gap carried over rather than fixed
 *
 * `RequestReplayResult` declares `durationMs`, and `replayRequest` does not return it — so this
 * command's result has never carried the field its own frozen contract advertises. It is moved as-is for
 * the same reason `configCommands.ts` left `workspace.add`'s odd failure shape alone: changing a
 * command's result shape while moving it hides the move behind a contract change. The renderer does not
 * read it (it times the call itself — see `restTabReducer.ts`'s own `durationMs`), so this is latent, and
 * it is recorded here rather than silently patched.
 */
import type { RequestReplayParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { replayRequest } from "../utils/replay";

export function registerRestCommands(registry: CommandRegistry): void {
    registry.register("request.replay", (params: RequestReplayParams) =>
        replayRequest(params.method, params.url, params.headers, params.body),
    );
}
