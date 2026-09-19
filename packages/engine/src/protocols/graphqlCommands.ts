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
 * for (`miscCommands.ts`), and the same fix: import the builtins statically. They are Node builtins
 * with no cycle and no cost, and `proxy/serverReplay.ts` / `proxy/webhookServer.ts` already do it.
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
 */
import * as http from "http";
import * as https from "https";
import type { GraphqlIntrospectParams, GraphqlExecuteParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";

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
}
