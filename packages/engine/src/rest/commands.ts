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
import type { CaptureBuildRequestParams, RequestParseCurlParams, RequestReplayParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import type { RequestLogEntry } from "../kernel/telemetry";
import { replayRequest } from "../utils/replay";
import { parseCurl } from "../utils/curl";
import { capturedToRequest } from "../capture/captureEntities";

export function registerRestCommands(registry: CommandRegistry): void {
    registry.register("request.replay", (params: RequestReplayParams) =>
        replayRequest(params.method, params.url, params.headers, params.body),
    );

    /**
     * `plan/19` WI 5 — the cURL parser, on the wire.
     *
     * The renderer used to own this (`packages/gui/src/lib/curlParser.ts`) and the cURL importer owned a
     * near-verbatim second copy; both are gone and the parser is `utils/curl.ts`. The command exists
     * because the renderer cannot import the engine (`plan/19` finding 1 — `@bifurc/gui` has no
     * `@bifurc/engine` dependency by design), so the *only* way for the REST tab's import field to parse a
     * pasted command is to ask the engine to.
     *
     * **It returns the parse unfiltered.** `SKIP_CURL_HEADERS` is a policy decision the *caller* makes
     * (the tab drops hop-by-hop headers on import; the importer drops them at its own point in the
     * pipeline), so a command that dropped them would be choosing for both. See `utils/curl.ts`.
     */
    registry.register("request.parseCurl", (params: RequestParseCurlParams) => parseCurl(params.curl));

    /**
     * `capture.buildRequest` — `plan/19` WI 5, item 2. It lives here, not in a capture module, for the
     * reason `request.send` lives with the pipeline: the thing it builds is a **`requests` entity**, so it
     * belongs to the feature that owns that entity. The construction itself is in
     * `utils/captureEntities.ts`, which reads nothing and is shared with `mocks/commands.ts`.
     *
     * `entry` is loose on the wire (`Record<string, unknown>`), so the cast is the boundary's, not a
     * shortcut — `@bifurc/protocol` deliberately does not model `RequestLogEntry` (`misc.ts` says why).
     */
    registry.register("capture.buildRequest", (params: CaptureBuildRequestParams) => ({
        request: capturedToRequest(params.entry as unknown as RequestLogEntry),
    }));
}
