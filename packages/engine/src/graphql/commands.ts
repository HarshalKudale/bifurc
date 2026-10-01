/**
 * `graphql.introspect` / `graphql.execute` — the two GraphQL request commands, moved out of
 * `src/ipc/handlers/graphqlHandlers.ts` (P6 step 3e, slice 4).
 *
 * ## Why they moved (§5c)
 *
 * They were registered at **module scope** in the shell file, so a containerised engine (P9) answered
 * `UNKNOWN_COMMAND` for both. `graphqlHandlers.ts`'s channel adapters are thin
 * `commandRegistry.invoke()` delegations, so in the shell these two channels were only ever as alive as
 * the registry behind them — and on a headless host there was no registry behind them at all.
 *
 * The rest of that file — `graphql:addRequest/updateRequest/deleteRequest`,
 * `graphql:addMock/updateMock/deleteMock` and `graphql:addSchema/deleteSchema/listSchemas` — stays
 * where it is: those route through `registerEntityCrudHandlers()` /
 * `registerSimpleEntityHandlers()`, which are the shell's CRUD collapse, not module-scope command
 * registrations. §5c counts `^commandRegistry.register(` only, and these two were the only ones.
 *
 * ## `require` → static import, and this is not cosmetic
 *
 * Both bodies reached for their transport with `require("https")` / `require("http")` *inside the
 * handler*. That works in the shell, which is CJS — but `require` **does not exist in this package's
 * ESM build output**, so a verbatim copy would have compiled, typechecked and passed every unit test,
 * then thrown at *call* time in `dist/*.mjs`. This is the same defect `healthbar.checkUrl` was fixed
 * for (`workspace/healthbarCommands.ts`), and the same fix: import the builtins statically. They are
 * Node builtins with no cycle and no cost, and `utils/replay.ts` / `webhooks/webhookServer.ts` do it.
 *
 * ## The two commands fail in two different ways, and both are load-bearing
 *
 * `graphql.introspect` **catches** and resolves `{ok:false, error}`; `graphql.execute` **throws**.
 * That asymmetry is the shell's and is preserved verbatim. It matters because the transport hands a
 * handler's value back *uninspected*: a resolved `{ok:false}` is an envelope **success** carrying a
 * failed result, while a throw becomes an envelope `{ok:false}` with a transport-level error code.
 * So the two are not interchangeable, and "the command reports failure" describes only one of them.
 *
 * `graphql.introspect` also returns the raw response body under `sdl` and does **not** parse it —
 * despite the name it does no schema decoding at all, it posts the introspection query and hands back
 * whatever came out. The name is the renderer's, and the renderer does the parsing.
 *
 * `variables` is a JSON **string** in the payload (the request editor holds it as text). A malformed
 * one is swallowed and left `undefined` rather than failing the request — silent, and preserved.
 *
 * ## `graphql.send` — the third command, and the first one that resolves (2026-09-30)
 *
 * `plan/19` WI 3's second protocol. It replaced `graphql.executeSaved`, whose params were
 * `{workspaceId, id}`; the new params are a **union** of that form and `{workspaceId, request}`, so a draft
 * tab and the CLI use one command. `plan/protocol-changes.md`'s 2026-09-30 `graphql.send` entry owns the wire
 * argument; this docblock owns the behaviour.
 *
 * **It resolves every field and runs no scripts**, and both halves are deliberate:
 *
 * - *Resolves every field* — `endpointUrl`, `headers`, `query`, `variables` **and** `operationName`, against
 *   the workspace's merged environment (`workspace/envState.ts`, D23). This is a **fix**, not a
 *   re-implementation: the tab resolved `endpointUrl`, `headers` and `variables` but **not** `query` or
 *   `operationName`, while the Send gate's token scan treated all five as resolvable — so a `{{token}}` in a
 *   query never resolved in *either* path. The gate was written from intent and the tab from practice, and
 *   the engine follows the gate, which is the only form under which `usesVariables` can leave the gate at
 *   all (`plan/19` §7).
 * - *No scripts* — the sandbox's context is a `{method, url, headers, body}` request and a GraphQL request is
 *   not one, so `graphql.send` has no `scripts` member rather than one nothing could run.
 *
 * It lives here rather than in `workspace/entities/savedRequestCommands.ts` because it reads `workspace/` +
 * `utils/` and dispatches through `kernel/` — this module's §6 row exactly — while the two counterparts that
 * remain there are pure load-and-dispatch.
 */
import * as http from "http";
import * as https from "https";
import { EngineError } from "@bifurc/protocol";
import type {
    GraphqlIntrospectParams,
    GraphqlExecuteParams,
    GraphqlSendParams,
} from "@bifurc/protocol";
import type { CommandContext, CommandRegistry } from "../kernel/registry";
import { loadEntity } from "../workspace/config";
import { loadMergedEnvironment } from "../env/envState";
import { resolveVars, resolveHeaders } from "../env/resolve";

/**
 * The standard introspection query, verbatim from the shell. It is written out in full rather than
 * built from a library because the engine has no GraphQL dependency — this string is the whole reason
 * `graphql.introspect` can work at all.
 */
const INTROSPECTION_QUERY = `
      query IntrospectionQuery {
        __schema {
          queryType { name }
          mutationType { name }
          subscriptionType { name }
          types { ...FullType }
          directives { name description locations args { ...InputValue } }
        }
      }
      fragment FullType on __Type {
        kind name description
        fields(includeDeprecated: true) { name description args { ...InputValue } type { ...TypeRef } isDeprecated deprecationReason }
        inputFields { ...InputValue }
        interfaces { ...TypeRef }
        enumValues(includeDeprecated: true) { name description isDeprecated deprecationReason }
        possibleTypes { ...TypeRef }
      }
      fragment InputValue on __InputValue { name description type { ...TypeRef } defaultValue }
      fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } } } }
    `;

export function registerGraphqlCommands(registry: CommandRegistry): void {
    registry.register("graphql.introspect", async ({ url, headers }: GraphqlIntrospectParams) => {
        try {
            const httpMod = url.startsWith("https") ? https : http;
            const body = JSON.stringify({ query: INTROSPECTION_QUERY });
            const parsed = new URL(url);
            const reqHeaders: Record<string, string> = { "Content-Type": "application/json", "Content-Length": String(Buffer.byteLength(body)), ...headers };
            const result = await new Promise<string>((resolve, reject) => {
                const req = httpMod.request({ hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "POST", headers: reqHeaders, rejectUnauthorized: false }, (res: any) => {
                    let data = "";
                    res.on("data", (chunk: string) => { data += chunk; });
                    res.on("end", () => resolve(data));
                });
                req.on("error", reject);
                req.write(body);
                req.end();
            });
            return { ok: true, sdl: result };
        } catch (err: any) {
            return { ok: false, error: err.message ?? String(err) };
        }
    });

    registry.register("graphql.execute", async ({ url, headers, query, variables, operationName }: GraphqlExecuteParams) => {
        const httpMod = url.startsWith("https") ? https : http;
        let parsedVars: unknown = undefined;
        try { if (variables && variables.trim()) parsedVars = JSON.parse(variables); } catch { /* leave undefined */ }
        const body = JSON.stringify({ query, variables: parsedVars, operationName: operationName || undefined });
        const parsed = new URL(url);
        const reqHeaders: Record<string, string> = { "Content-Type": "application/json", "Content-Length": String(Buffer.byteLength(body)), ...headers };
        const start = Date.now();
        const { status, resHeaders, resBody } = await new Promise<{ status: number; resHeaders: Record<string, string>; resBody: string }>((resolve, reject) => {
            const req = httpMod.request({ hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "POST", headers: reqHeaders, rejectUnauthorized: false }, (res: any) => {
                let data = "";
                res.on("data", (chunk: string) => { data += chunk; });
                res.on("end", () => {
                    const h: Record<string, string> = {};
                    for (const [k, v] of Object.entries(res.headers)) { h[k] = Array.isArray(v) ? v.join(", ") : String(v); }
                    resolve({ status: res.statusCode ?? 0, resHeaders: h, resBody: data });
                });
            });
            req.on("error", reject);
            req.write(body);
            req.end();
        });
        return { status, headers: resHeaders, body: resBody, durationMs: Date.now() - start };
    });

    registry.register("graphql.send", async (params: GraphqlSendParams, ctx: CommandContext) => {
        const stored = "id" in params
            ? requireStoredGraphqlRequest(params.workspaceId, params.id)
            : params.request;
        // One read of the workspace's environment state for the whole send. `plan/19` D23: the engine is the
        // authority for which environment a request resolves against, and the merge is part of that.
        const env = loadMergedEnvironment(params.workspaceId);
        return registry.invoke(
            "graphql.execute",
            {
                // `endpointUrl` → `url` is the only mapping, and it is why this file knows the stored shape
                // at all. **Every** value is resolved — including `query` and `operationName`, which the tab
                // never resolved while the Send gate treated them as resolvable (see the header).
                url: resolveVars(stored.endpointUrl, env),
                headers: resolveHeaders(stored.headers ?? {}, env),
                query: resolveVars(stored.query, env),
                variables: resolveVars(stored.variables, env),
                operationName: resolveVars(stored.operationName, env),
            },
            ctx,
        );
    });
}

/**
 * The stored entity's fields, as a **projection** — exactly the ones the send reads.
 *
 * Declared *after* its only reader, which is legal for a type and keeps this file's shape "the three
 * commands, then what they read". A field the send does not use is deliberately absent, so it cannot quietly
 * become required here. The canonical declaration lives beside the `graphqlRequests` kind in
 * `graphql/kinds.ts` and is not re-exported, because nothing outside those two files had ever needed it.
 * (This projection lived in `workspace/entities/savedRequestCommands.ts` until 2026-09-30; it moved with the
 * command, which is its only reader.)
 */
interface StoredGraphqlRequest {
    endpointUrl: string;
    headers?: Record<string, string>;
    query: string;
    variables: string;
    operationName: string;
}

/**
 * Load the entity or fail loudly, rather than answering a failure-shaped value.
 *
 * `HttpLikeResult` has **no** `ok` or `error` field, so a `{ok:false}`-shaped result would be a lie about the
 * declared contract — and the transport hands a handler's value back *uninspected*, so the client would read
 * `status: undefined` off a successful envelope. So this throws, which the transport preserves as the error
 * code and which `@bifurc/client`'s retry policy treats as non-retryable — correctly, since the entity is not
 * going to appear on a second attempt. Same reasoning and same behaviour as the two counterparts still in
 * `workspace/entities/savedRequestCommands.ts`.
 */
function requireStoredGraphqlRequest(workspaceId: string, id: string): StoredGraphqlRequest {
    const entity = loadEntity<StoredGraphqlRequest>(workspaceId, "graphqlRequests", id);
    if (!entity) {
        throw new EngineError(
            "NOT_FOUND",
            `No saved graphqlRequests entity "${id}" in workspace "${workspaceId}".`,
        );
    }
    return entity;
}
