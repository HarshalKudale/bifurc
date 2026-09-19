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
 * (`miscCommands.ts`); same fix, static builtin imports.
 *
 * ## The two commands fail in two different ways, and both are load-bearing
 *
 * `soap.fetchWsdl` **catches** and resolves `{ok:false, error}`; `soap.execute` **throws**. That
 * asymmetry is the shell's and is preserved verbatim, because the transport hands a handler's value
 * back *uninspected*: a resolved `{ok:false}` is an envelope **success** carrying a failed result,
 * while a throw becomes an envelope `{ok:false}` with a transport-level error code. `soap.execute`
 * against a dead endpoint therefore surfaces as `[INTERNAL] connect ECONNREFUSED …` rather than as a
 * result object — which is exactly what `tests/spike/protocolPoc.test.ts` pins, and why that one test
 * is environment-sensitive: the sandbox it was written in answered `127.0.0.1:1` with a synthetic 404
 * instead of refusing, so the request *succeeded* and the body carried a numeric `status`. On a host
 * that actually refuses, the same assertion fails on a `ProtocolClientError`. Recorded in
 * `plan/baseline.md` "Environment caveats" and left as-is: changing which of the two shapes
 * `soap.execute` uses would be a contract change, and the point the test exists to make — the
 * envelope never throws — holds either way.
 *
 * ## `startsWith("https")`, kept rather than "corrected"
 *
 * The scheme test is a string prefix check, not `new URL(url).protocol === "https:"`. The two differ
 * for a URL like `httpsfoo://…`. Kept verbatim: this is a move, and quietly tightening a scheme
 * check changes which requests get TLS. `rejectUnauthorized: false` is also deliberate and kept —
 * these are the user's own endpoints, and a self-signed certificate is the norm for a development
 * SOAP server.
 */
import * as http from "http";
import * as https from "https";
import type { SoapFetchWsdlParams, SoapExecuteParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";

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
}
