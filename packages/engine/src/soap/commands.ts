/**
 * `soap.fetchWsdl` / `soap.execute` — the two SOAP request commands, moved out of
 * `src/ipc/handlers/soapHandlers.ts` (P6 step 3e, slice 4).
 *
 * ## Why they moved (§5c)
 *
 * They were registered at **module scope** in the shell file, so a containerised engine (P9) answered
 * `UNKNOWN_COMMAND` for both. `soapHandlers.ts`'s channel adapters are thin
 * `commandRegistry.invoke()` delegations, so these two channels were only ever as alive as the
 * registry behind them.
 *
 * The rest of that file — `soap:addRequest/updateRequest/deleteRequest`,
 * `soap:addMock/updateMock/deleteMock` and `soap:addWsdl/deleteWsdl/listWsdls` — stays where it is:
 * those route through the shell's CRUD collapse (`registerEntityCrudHandlers()` /
 * `registerSimpleEntityHandlers()`), not module-scope registrations, so §5c's grep never counted them.
 *
 * ## `require` → static import, and this is not cosmetic
 *
 * Both bodies reached for their transport with `require("https")` / `require("http")` *inside the
 * handler*. That works in the shell, which is CJS — but `require` **does not exist in this package's
 * ESM build output**, so a verbatim copy would have compiled, typechecked and passed every unit test,
 * then thrown at *call* time in `dist/*.mjs`. Same defect `healthbar.checkUrl` was fixed for
 * (`workspace/healthbarCommands.ts`); same fix, static builtin imports.
 *
 * ## The two commands fail in two different ways, and both are load-bearing
 *
 * `soap.fetchWsdl` **catches** and resolves `{ok:false, error}`; `soap.execute` **throws**. That
 * asymmetry is the shell's and is preserved verbatim, because the transport hands a handler's value
 * back *uninspected*: a resolved `{ok:false}` is an envelope **success** carrying a failed result,
 * while a throw becomes an envelope `{ok:false}` with a transport-level error code. `soap.execute`
 * against a dead endpoint therefore surfaces as `[INTERNAL] connect ECONNREFUSED …` rather than as a
 * result object.
 *
 * **What pins it, and a correction worth keeping.** This used to cite `tests/spike/protocolPoc.test.ts`,
 * which asserted the same asymmetry against `127.0.0.1:1` and was environment-sensitive: the sandbox it
 * was written in answered that address with a synthetic 404 instead of refusing, so the request
 * *succeeded* and the assertion depended on which answer the host gave — which is why it was the suite's
 * one standing failure. **The spike was retired 2026-09-21** (`spike/README.md`), and the contract moved
 * to `tests/integration/protocolExecution.integration.test.ts`, which asserts the same two shapes using
 * `deadPort()` — a real dead port, hermetic on every host. **Same contract, firmer fixture.** Recorded in
 * `plan/baseline.md` "Environment caveats": changing which of the two shapes `soap.execute` uses would be
 * a contract change, and the point the test exists to make — the envelope never throws — holds either way.
 * The sandbox's synthetic 404 remains true *of that sandbox*, which is what the baseline note records.
 *
 * ## `startsWith("https")`, kept rather than "corrected"
 *
 * The scheme test is a string prefix check, not `new URL(url).protocol === "https:"`. The two differ
 * for a URL like `httpsfoo://…`. Kept verbatim: this is a move, and quietly tightening a scheme
 * check changes which requests get TLS. `rejectUnauthorized: false` is also deliberate and kept —
 * these are the user's own endpoints, and a self-signed certificate is the norm for a development
 * SOAP server.
 *
 * ## `soap.send` — the third command, and the second that resolves (2026-09-30)
 *
 * `plan/19` WI 3's third protocol. It replaced `soap.executeSaved`, whose params were `{workspaceId, id}`;
 * the new params are a **union** of that form and `{workspaceId, request}`, so a draft tab and the CLI use one
 * command. `plan/protocol-changes.md`'s 2026-09-30 `soap.send` entry owns the wire argument.
 *
 * **It resolves every field and runs no scripts**, the same shape as `graphql.send` and for the same reasons:
 *
 * - *Every field* — `endpointUrl`, `soapAction`, `headers` **and** `body`, against the workspace's merged
 *   environment (`workspace/envState.ts`, D23). The tab resolved all of these **except `soapAction`**, while
 *   the Send gate's token scan treated all four as resolvable — so a `{{token}}` in a SOAPAction never
 *   resolved in *either* path. The gate was written from intent and the tab from practice, and the engine
 *   follows the gate.
 * - *No scripts* — the sandbox's context is a `{method, url, headers, body}` request and a SOAP request is not
 *   one, so `soap.send` has no `scripts` member rather than one nothing could run.
 *
 * It lives here rather than in `workspace/entities/savedRequestCommands.ts` because it reads `workspace/` +
 * `utils/` and dispatches through `kernel/` — this module's §6 row exactly — while the one counterpart that
 * remains there is pure load-and-dispatch.
 */
import * as http from "http";
import * as https from "https";
import { EngineError } from "@bifurc/protocol";
import type { SoapFetchWsdlParams, SoapExecuteParams, SoapSendParams } from "@bifurc/protocol";
import type { CommandContext, CommandRegistry } from "../kernel/registry";
import { loadEntity } from "../workspace/config";
import { loadMergedEnvironment } from "../env/envState";
import { resolveVars, resolveHeaders } from "../env/resolve";

export function registerSoapCommands(registry: CommandRegistry): void {
    registry.register("soap.fetchWsdl", async ({ url }: SoapFetchWsdlParams) => {
        const httpMod = url.startsWith("https") ? https : http;
        try {
            const parsed = new URL(url);
            const content = await new Promise<string>((resolve, reject) => {
                const req = httpMod.request(
                    { hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "GET", rejectUnauthorized: false },
                    (res: any) => {
                        let data = "";
                        res.on("data", (chunk: string) => { data += chunk; });
                        res.on("end", () => resolve(data));
                    },
                );
                req.on("error", reject);
                req.end();
            });
            return { ok: true, content };
        } catch (err: any) {
            return { ok: false, error: err?.message ?? "Failed to fetch WSDL" };
        }
    });

    registry.register("soap.execute", async ({ endpointUrl, soapAction, headers, body }: SoapExecuteParams) => {
        const httpMod = endpointUrl.startsWith("https") ? https : http;
        const parsed = new URL(endpointUrl);
        const xmlBody = Buffer.from(body, "utf-8");
        const reqHeaders: Record<string, string> = {
            "Content-Type": "text/xml; charset=utf-8",
            "Content-Length": String(xmlBody.length),
            ...headers,
        };
        if (soapAction) reqHeaders["SOAPAction"] = soapAction;
        const start = Date.now();
        const { status, resHeaders, resBody } = await new Promise<{ status: number; resHeaders: Record<string, string>; resBody: string }>((resolve, reject) => {
            const req = httpMod.request(
                { hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "POST", headers: reqHeaders, rejectUnauthorized: false },
                (res: any) => {
                    let data = "";
                    res.on("data", (chunk: string) => { data += chunk; });
                    res.on("end", () => {
                        const h: Record<string, string> = {};
                        for (const [k, v] of Object.entries(res.headers)) { h[k] = Array.isArray(v) ? v.join(", ") : String(v); }
                        resolve({ status: res.statusCode ?? 0, resHeaders: h, resBody: data });
                    });
                },
            );
            req.on("error", reject);
            req.write(xmlBody);
            req.end();
        });
        return { status, headers: resHeaders, body: resBody, durationMs: Date.now() - start };
    });

    registry.register("soap.send", async (params: SoapSendParams, ctx: CommandContext) => {
        const stored = "id" in params
            ? requireStoredSoapRequest(params.workspaceId, params.id)
            : params.request;
        // One read of the workspace's environment state for the whole send. `plan/19` D23: the engine is the
        // authority for which environment a request resolves against, and the merge is part of that.
        const env = loadMergedEnvironment(params.workspaceId);
        return registry.invoke(
            "soap.execute",
            {
                // Field-for-field identical to the stored shape, so there is no rename here — but **every**
                // field is resolved, including `soapAction`, which the tab never resolved while the Send gate
                // treated it as resolvable (see the header).
                endpointUrl: resolveVars(stored.endpointUrl, env),
                soapAction: resolveVars(stored.soapAction, env),
                headers: resolveHeaders(stored.headers ?? {}, env),
                body: resolveVars(stored.body, env),
            },
            ctx,
        );
    });
}

/**
 * The stored entity's fields, as a **projection** — exactly the ones the send reads.
 *
 * Declared *after* its only reader, which is legal for a type and keeps this file's shape "the three
 * commands, then what they read". The canonical declaration lives beside the `soapRequests` kind in
 * `soap/kinds.ts` and is not re-exported, because nothing outside those two files had ever needed it. (This
 * projection lived in `workspace/entities/savedRequestCommands.ts` until 2026-09-30; it moved with the
 * command, which is its only reader.)
 */
interface StoredSoapRequest {
    endpointUrl: string;
    soapAction: string;
    headers?: Record<string, string>;
    body: string;
}

/**
 * Load the entity or fail loudly, rather than answering a failure-shaped value.
 *
 * `SoapExecuteResult` is `HttpLikeResult`, which has **no** `ok` or `error` field — so a `{ok:false}`-shaped
 * result would be a lie about the declared contract, and the transport hands a handler's value back
 * *uninspected*, leaving the client to read `status: undefined` off a successful envelope. So this throws,
 * which the transport preserves as the error code and which `@bifurc/client`'s retry policy treats as
 * non-retryable — correctly, since the entity is not going to appear on a second attempt. Same reasoning and
 * same behaviour as the counterpart still in `workspace/entities/savedRequestCommands.ts`.
 */
function requireStoredSoapRequest(workspaceId: string, id: string): StoredSoapRequest {
    const entity = loadEntity<StoredSoapRequest>(workspaceId, "soapRequests", id);
    if (!entity) {
        throw new EngineError(
            "NOT_FOUND",
            `No saved soapRequests entity "${id}" in workspace "${workspaceId}".`,
        );
    }
    return entity;
}
